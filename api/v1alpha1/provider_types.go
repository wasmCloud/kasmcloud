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

// ProviderSpec defines the desired state of Provider
type ProviderSpec struct {
	// +kubebuilder:validation:MinLength:=1
	Host string `json:"host"`

	// +kubebuilder:validation:MinLength:=1
	Image string `json:"image"`

	// +kubebuilder:validation:MinLength:=1
	Link string `json:"link"`
}

// ProviderStatus defines the observed state of Provider
type ProviderStatus struct {
	PublicKey       string `json:"publicKey"`
	ContractId      string `json:"contractId"`
	DescriptiveName string `json:"descriptiveName,omitempty"`

	Claims              Claims   `json:"claims"`
	ArchitectureTargets []string `json:"architectureTargets"`

	Vendor    string  `json:"vendor"`
	Version   *string `json:"version,omitempty"`
	Reversion *int    `json:"reversion,omitempty"`

	Conditions []metav1.Condition `json:"conditions"`
	InstanceId string             `json:"instanceId"`
}

// +kubebuilder:object:root=true
// +kubebuilder:resource:path=providers,scope=Namespaced,categories=kasmcloud
// +kubebuilder:subresource:status
// +kubebuilder:printcolumn:name="Desc",type="string",JSONPath=".status.descriptiveName"
// +kubebuilder:printcolumn:name="PublicKey",type="string",JSONPath=".status.publicKey"
// +kubebuilder:printcolumn:name="Link",type="string",JSONPath=".spec.link"
// +kubebuilder:printcolumn:name="ContractId",type="string",JSONPath=".status.contractId"
// +kubebuilder:printcolumn:name="Image",type="string",JSONPath=".spec.image"

// Provider is the Schema for the providers API
type Provider struct {
	metav1.TypeMeta   `json:",inline"`
	metav1.ObjectMeta `json:"metadata,omitempty"`

	Spec   ProviderSpec   `json:"spec,omitempty"`
	Status ProviderStatus `json:"status,omitempty"`
}

//+kubebuilder:object:root=true

// ProviderList contains a list of Provider
type ProviderList struct {
	metav1.TypeMeta `json:",inline"`
	metav1.ListMeta `json:"metadata,omitempty"`
	Items           []Provider `json:"items"`
}

func init() {
	SchemeBuilder.Register(&Provider{}, &ProviderList{})
}
