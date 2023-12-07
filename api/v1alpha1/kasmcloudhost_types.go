/*
Copyright 2023.

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

package v1alpha1

import (
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// EDIT THIS FILE!  THIS IS SCAFFOLDING FOR YOU TO OWN!
// NOTE: json tags are required.  Any new fields you add must have json tags for the fields to be serialized.

// KasmCloudHostSpec defines the desired state of KasmCloudHost
type KasmCloudHostSpec struct {
	// INSERT ADDITIONAL SPEC FIELDS - desired state of cluster
	// Important: Run "make" to regenerate code after modifying this file

	ClusterIssuers []string `json:"clusterIssuers,omitempty"`
}

// KasmCloudHostStatus defines the observed state of KasmCloudHost
type KasmCloudHostStatus struct {
	KubeNodeName string `json:"kubeNodeName,omitempty"`
	Instance     string `json:"instance,omitempty"`
	PreInstance  string `json:"preInstance,omitempty"`

	PublicKey        string   `json:"publicKey"`
	ClusterPublicKey string   `json:"clusterPublicKey"`
	ClusterIssuers   []string `json:"clusterIssuers"`

	Providers KasmCloudHostProviderStatus `json:"providers"`
	Actors    KasmCloudHostActorStatus    `json:"actors"`
}

type KasmCloudHostProviderStatus struct {
	Count uint64 `json:"count"`
}

type KasmCloudHostActorStatus struct {
	Count         uint64 `json:"count"`
	InstanceCount uint64 `json:"instanceCount"`
}

//+kubebuilder:object:root=true
//+kubebuilder:subresource:status

// KasmCloudHost is the Schema for the kasmcloudhosts API
type KasmCloudHost struct {
	metav1.TypeMeta   `json:",inline"`
	metav1.ObjectMeta `json:"metadata,omitempty"`

	Spec   KasmCloudHostSpec   `json:"spec,omitempty"`
	Status KasmCloudHostStatus `json:"status,omitempty"`
}

//+kubebuilder:object:root=true

// KasmCloudHostList contains a list of KasmCloudHost
type KasmCloudHostList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []KasmCloudHost `json:"items"`
}

func init() {
	SchemeBuilder.Register(&KasmCloudHost{}, &KasmCloudHostList{})
}
